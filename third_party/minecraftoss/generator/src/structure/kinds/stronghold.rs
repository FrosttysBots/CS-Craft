//! Strongholds (vanilla `StrongholdStructure`, `StrongholdPieces`).
//!
//! Source-informed from the pinned 26.3 JAR. A layout grows from a spiral
//! staircase whose first child is always a five-way crossing; children are
//! expanded from a pending list in random order, and a piece choice that
//! does not fit keeps trying every later weight entry (the selection stays
//! negative), as vanilla does. Layouts without a portal room are discarded
//! and regrown from a reseeded random until one has it.

use crate::feature::template::BoundingBox;
use crate::feature::Ctx;
use crate::structure::piece::{BlockSelector, Piece, PieceBase};
use crate::structure::placement::large_feature_seed;
use crate::structure::{GenerationContext, PieceList, StructureKind, Stub};
use minecraftoss_core::pos::Direction;
use minecraftoss_core::random::{LegacyRandom, RandomSource, WorldgenRandom};
use minecraftoss_core::{BlockPos, BlockStateId, ChunkPos};

#[derive(Debug)]
pub struct Stronghold;

impl StructureKind for Stronghold {
    fn find_generation_point<'s>(&'s self, ctx: &mut GenerationContext) -> Option<Stub<'s>> {
        let chunk = ctx.chunk;
        let position = BlockPos::new(chunk.min_block_x(), 0, chunk.min_block_z());
        Some(Stub::deferred(position, move |ctx: &mut GenerationContext| generate(ctx)))
    }
}

/// `StrongholdPiece.SmallDoorType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Door {
    Opening,
    WoodDoor,
    Grates,
    IronDoor,
}

/// `StrongholdPiece.randomSmallDoor`.
fn random_door(random: &mut impl RandomSource) -> Door {
    match random.next_i32_bound(5) {
        2 => Door::WoodDoor,
        3 => Door::Grates,
        4 => Door::IronDoor,
        _ => Door::Opening,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// `StartPiece`: a source `StairsDown`.
    Start,
    ChestCorridor { placed_chest: bool },
    FillerCorridor { steps: i32 },
    FiveCrossing { left_low: bool, left_high: bool, right_low: bool, right_high: bool },
    LeftTurn,
    Library { tall: bool },
    PortalRoom { placed_spawner: bool },
    PrisonHall,
    RightTurn,
    RoomCrossing { room: i32 },
    StairsDown,
    Straight { left: bool, right: bool },
    StraightStairsDown,
}

/// Which piece a weight entry makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Straight,
    PrisonHall,
    LeftTurn,
    RightTurn,
    RoomCrossing,
    StraightStairsDown,
    StairsDown,
    FiveCrossing,
    ChestCorridor,
    Library,
    PortalRoom,
}

/// `StrongholdPieces.PieceWeight`.
#[derive(Clone, Copy, Debug)]
struct Weight {
    choice: Choice,
    weight: i32,
    place_count: i32,
    max_place_count: i32,
}

impl Weight {
    const fn new(choice: Choice, weight: i32, max_place_count: i32) -> Self {
        Self { choice, weight, place_count: 0, max_place_count }
    }

    fn is_valid(&self) -> bool {
        self.max_place_count == 0 || self.place_count < self.max_place_count
    }

    /// `doPlace`, with the library and portal room's depth floors.
    fn do_place(&self, depth: i32) -> bool {
        self.is_valid()
            && match self.choice {
                Choice::Library => depth > 4,
                Choice::PortalRoom => depth > 5,
                _ => true,
            }
    }
}

const WEIGHTS: [Weight; 11] = [
    Weight::new(Choice::Straight, 40, 0),
    Weight::new(Choice::PrisonHall, 5, 5),
    Weight::new(Choice::LeftTurn, 20, 0),
    Weight::new(Choice::RightTurn, 20, 0),
    Weight::new(Choice::RoomCrossing, 10, 6),
    Weight::new(Choice::StraightStairsDown, 5, 5),
    Weight::new(Choice::StairsDown, 5, 5),
    Weight::new(Choice::FiveCrossing, 5, 4),
    Weight::new(Choice::ChestCorridor, 5, 4),
    Weight::new(Choice::Library, 10, 2),
    Weight::new(Choice::PortalRoom, 20, 1),
];

#[derive(Debug)]
pub struct StrongholdPiece {
    base: PieceBase,
    kind: Kind,
    entry_door: Door,
}

fn oriented(depth: i32, bbox: BoundingBox, direction: Direction, kind: Kind, entry_door: Door) -> StrongholdPiece {
    let mut base = PieceBase::new(depth, bbox);
    base.set_orientation(Some(direction));
    StrongholdPiece { base, kind, entry_door }
}

/// The layout under construction: the builder, the static weight state
/// vanilla resets per attempt, and the `StartPiece` fields.
struct Layout {
    pieces: Vec<StrongholdPiece>,
    pending: Vec<usize>,
    weights: [Weight; 11],
    /// `currentPieces`, as indices into `weights`.
    current: Vec<usize>,
    imposed: Option<Choice>,
    previous: Option<usize>,
    portal_room: bool,
}

/// `StrongholdStructure.generatePieces`.
fn generate(ctx: &mut GenerationContext) -> PieceList {
    let chunk = ctx.chunk;
    let (sea_level, min_y) = (ctx.terrain.sea_level, ctx.terrain.min_y);
    let random = &mut ctx.random;
    let mut tries: i64 = 0;
    loop {
        large_feature_seed(random, ctx.seed.wrapping_add(tries), chunk.x, chunk.z);
        tries += 1;
        let direction = PieceBase::random_horizontal_direction(random);
        let bbox = PieceBase::make_bounding_box(chunk.min_block_x() + 2, 64, chunk.min_block_z() + 2, direction, 5, 11, 5);
        let mut layout = Layout {
            pieces: vec![oriented(0, bbox, direction, Kind::Start, Door::Opening)],
            pending: Vec::new(),
            weights: WEIGHTS,
            current: (0..WEIGHTS.len()).collect(),
            imposed: None,
            previous: None,
            portal_room: false,
        };
        add_children(&mut layout, 0, random);
        while !layout.pending.is_empty() {
            let pos = random.next_i32_bound(layout.pending.len() as i32) as usize;
            let piece = layout.pending.remove(pos);
            add_children(&mut layout, piece, random);
        }
        // `moveBelowSeaLevel(seaLevel, minY, random, 10)`.
        let bounds = BoundingBox::encapsulating_all(layout.pieces.iter().map(|p| &p.base.bbox)).expect("stronghold start");
        let max_y = sea_level - 10;
        let mut y1 = bounds.y_span() + min_y + 1;
        if y1 < max_y {
            y1 += random.next_i32_bound(max_y - y1);
        }
        let dy = y1 - bounds.max_y;
        if layout.portal_room {
            return layout
                .pieces
                .into_iter()
                .map(|mut p| {
                    p.base.bbox = p.base.bbox.moved(0, dy, 0);
                    Box::new(p) as Box<dyn Piece>
                })
                .collect();
        }
    }
}

/// `isOkBox` and no collision with the pieces so far.
fn fits(layout: &Layout, b: &BoundingBox) -> bool {
    b.min_y > 10 && !layout.pieces.iter().any(|p| p.base.bbox.intersects(b))
}

/// `findAndCreatePieceFactory`: each constructor draws its door and
/// options only once its box fits.
fn create(layout: &Layout, choice: Choice, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<StrongholdPiece> {
    let (x, y, z) = foot;
    let make = |ox: i32, oy: i32, oz: i32, w: i32, h: i32, d: i32| BoundingBox::orient_box(x, y, z, ox, oy, oz, w, h, d, direction);
    let b = match choice {
        Choice::Straight | Choice::ChestCorridor => make(-1, -1, 0, 5, 5, 7),
        Choice::PrisonHall => make(-1, -1, 0, 9, 5, 11),
        Choice::LeftTurn | Choice::RightTurn => make(-1, -1, 0, 5, 5, 5),
        Choice::RoomCrossing => make(-4, -1, 0, 11, 7, 11),
        Choice::StraightStairsDown => make(-1, -7, 0, 5, 11, 8),
        Choice::StairsDown => make(-1, -7, 0, 5, 11, 5),
        Choice::FiveCrossing => make(-4, -3, 0, 10, 9, 11),
        Choice::PortalRoom => make(-4, -1, 0, 11, 8, 16),
        Choice::Library => {
            let tall = make(-4, -1, 0, 14, 11, 15);
            if fits(layout, &tall) {
                tall
            } else {
                make(-4, -1, 0, 14, 6, 15)
            }
        }
    };
    if !fits(layout, &b) {
        return None;
    }
    if choice == Choice::PortalRoom {
        return Some(oriented(depth, b, direction, Kind::PortalRoom { placed_spawner: false }, Door::Opening));
    }
    let door = random_door(random);
    let kind = match choice {
        Choice::Straight => {
            let left = random.next_i32_bound(2) == 0;
            let right = random.next_i32_bound(2) == 0;
            Kind::Straight { left, right }
        }
        Choice::PrisonHall => Kind::PrisonHall,
        Choice::LeftTurn => Kind::LeftTurn,
        Choice::RightTurn => Kind::RightTurn,
        Choice::RoomCrossing => Kind::RoomCrossing { room: random.next_i32_bound(5) },
        Choice::StraightStairsDown => Kind::StraightStairsDown,
        Choice::StairsDown => Kind::StairsDown,
        Choice::FiveCrossing => {
            let left_low = random.next_bool();
            let left_high = random.next_bool();
            let right_low = random.next_bool();
            let right_high = random.next_i32_bound(3) > 0;
            Kind::FiveCrossing { left_low, left_high, right_low, right_high }
        }
        Choice::ChestCorridor => Kind::ChestCorridor { placed_chest: false },
        Choice::Library => Kind::Library { tall: b.y_span() > 6 },
        Choice::PortalRoom => unreachable!(),
    };
    Some(oriented(depth, b, direction, kind, door))
}

/// `FillerCorridor.findPieceBox`: a corridor stub up to the first piece in
/// the way, when that piece sits at the same height.
fn filler_box(layout: &Layout, foot: (i32, i32, i32), direction: Direction) -> Option<BoundingBox> {
    let (x, y, z) = foot;
    let make = |depth: i32| BoundingBox::orient_box(x, y, z, -1, -1, 0, 5, 5, depth, direction);
    let b = make(4);
    let collision = layout.pieces.iter().find(|p| p.base.bbox.intersects(&b))?.base.bbox;
    if collision.min_y == b.min_y {
        for depth in [2, 1] {
            if !collision.intersects(&make(depth)) {
                return Some(make(depth + 1));
            }
        }
    }
    None
}

/// `generatePieceFromSmallDoor`.
fn generate_piece(layout: &mut Layout, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) -> Option<StrongholdPiece> {
    // `updatePieceWeight`.
    let any = layout.current.iter().any(|&i| {
        let w = &layout.weights[i];
        w.max_place_count > 0 && w.place_count < w.max_place_count
    });
    let total: i32 = layout.current.iter().map(|&i| layout.weights[i].weight).sum();
    if !any {
        return None;
    }
    if let Some(imposed) = layout.imposed.take() {
        if let Some(piece) = create(layout, imposed, random, foot, direction, depth) {
            return Some(piece);
        }
    }
    for _ in 0..5 {
        let mut selection = random.next_i32_bound(total);
        for slot in 0..layout.current.len() {
            let id = layout.current[slot];
            let w = layout.weights[id];
            selection -= w.weight;
            if selection < 0 {
                if !w.do_place(depth) || layout.previous == Some(id) {
                    break;
                }
                if let Some(piece) = create(layout, w.choice, random, foot, direction, depth) {
                    layout.weights[id].place_count += 1;
                    layout.previous = Some(id);
                    if !layout.weights[id].is_valid() {
                        layout.current.remove(slot);
                    }
                    return Some(piece);
                }
            }
        }
    }
    let b = filler_box(layout, foot, direction)?;
    (b.min_y > 1).then(|| {
        let steps = if matches!(direction, Direction::North | Direction::South) { b.z_span() } else { b.x_span() };
        oriented(depth, b, direction, Kind::FillerCorridor { steps }, Door::Opening)
    })
}

/// `generateAndAddPiece`.
fn generate_and_add(layout: &mut Layout, random: &mut LegacyRandom, foot: (i32, i32, i32), direction: Direction, depth: i32) {
    if depth > 50 {
        return;
    }
    let start = layout.pieces[0].base.bbox;
    if (foot.0 - start.min_x).abs() <= 112 && (foot.2 - start.min_z).abs() <= 112 {
        if let Some(piece) = generate_piece(layout, random, foot, direction, depth + 1) {
            layout.pieces.push(piece);
            layout.pending.push(layout.pieces.len() - 1);
        }
    }
}

fn child_forward(layout: &mut Layout, index: usize, random: &mut LegacyRandom, x_off: i32, y_off: i32) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let foot = match o {
        Direction::North => (b.min_x + x_off, b.min_y + y_off, b.min_z - 1),
        Direction::South => (b.min_x + x_off, b.min_y + y_off, b.max_z + 1),
        Direction::West => (b.min_x - 1, b.min_y + y_off, b.min_z + x_off),
        _ => (b.max_x + 1, b.min_y + y_off, b.min_z + x_off),
    };
    generate_and_add(layout, random, foot, o, depth);
}

fn child_left(layout: &mut Layout, index: usize, random: &mut LegacyRandom, y_off: i32, z_off: i32) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let (foot, direction) = match o {
        Direction::North | Direction::South => ((b.min_x - 1, b.min_y + y_off, b.min_z + z_off), Direction::West),
        _ => ((b.min_x + z_off, b.min_y + y_off, b.min_z - 1), Direction::North),
    };
    generate_and_add(layout, random, foot, direction, depth);
}

fn child_right(layout: &mut Layout, index: usize, random: &mut LegacyRandom, y_off: i32, z_off: i32) {
    let p = &layout.pieces[index].base;
    let (b, depth) = (p.bbox, p.gen_depth);
    let Some(o) = p.orientation() else { return };
    let (foot, direction) = match o {
        Direction::North | Direction::South => ((b.max_x + 1, b.min_y + y_off, b.min_z + z_off), Direction::East),
        _ => ((b.min_x + z_off, b.min_y + y_off, b.max_z + 1), Direction::South),
    };
    generate_and_add(layout, random, foot, direction, depth);
}

/// `addChildren` of the piece at `index`.
fn add_children(layout: &mut Layout, index: usize, random: &mut LegacyRandom) {
    let orientation = layout.pieces[index].base.orientation();
    let north_or_east = matches!(orientation, Some(Direction::North | Direction::East));
    match layout.pieces[index].kind {
        Kind::Start => {
            layout.imposed = Some(Choice::FiveCrossing);
            child_forward(layout, index, random, 1, 1);
        }
        Kind::StairsDown | Kind::ChestCorridor { .. } | Kind::PrisonHall | Kind::StraightStairsDown => child_forward(layout, index, random, 1, 1),
        Kind::FiveCrossing { left_low, left_high, right_low, right_high } => {
            let (mut a, mut b) = (3, 5);
            if matches!(orientation, Some(Direction::West | Direction::North)) {
                (a, b) = (8 - a, 8 - b);
            }
            child_forward(layout, index, random, 5, 1);
            if left_low {
                child_left(layout, index, random, a, 1);
            }
            if left_high {
                child_left(layout, index, random, b, 7);
            }
            if right_low {
                child_right(layout, index, random, a, 1);
            }
            if right_high {
                child_right(layout, index, random, b, 7);
            }
        }
        Kind::LeftTurn => {
            if north_or_east {
                child_left(layout, index, random, 1, 1);
            } else {
                child_right(layout, index, random, 1, 1);
            }
        }
        Kind::RightTurn => {
            if north_or_east {
                child_right(layout, index, random, 1, 1);
            } else {
                child_left(layout, index, random, 1, 1);
            }
        }
        Kind::RoomCrossing { .. } => {
            child_forward(layout, index, random, 4, 1);
            child_left(layout, index, random, 1, 4);
            child_right(layout, index, random, 1, 4);
        }
        Kind::Straight { left, right } => {
            child_forward(layout, index, random, 1, 1);
            if left {
                child_left(layout, index, random, 1, 2);
            }
            if right {
                child_right(layout, index, random, 1, 2);
            }
        }
        Kind::PortalRoom { .. } => layout.portal_room = true,
        Kind::FillerCorridor { .. } | Kind::Library { .. } => {}
    }
}

/// Block states the pieces use.
struct B {
    bricks: BlockStateId,
    cave_air: BlockStateId,
    brick_slab: BlockStateId,
    smooth_slab: BlockStateId,
    smooth_double: BlockStateId,
    wall_torch: BlockStateId,
    torch: BlockStateId,
    planks: BlockStateId,
    bookshelf: BlockStateId,
    cobweb: BlockStateId,
    fence: BlockStateId,
    ladder: BlockStateId,
    lava: BlockStateId,
    water: BlockStateId,
    bars: BlockStateId,
    brick_stairs: BlockStateId,
    frame: BlockStateId,
    portal: BlockStateId,
    spawner: BlockStateId,
    iron_door: BlockStateId,
    oak_door: BlockStateId,
    button: BlockStateId,
    cobblestone: BlockStateId,
    cobble_stairs: BlockStateId,
}

impl B {
    fn load(ctx: &Ctx) -> Self {
        let s = |name: &str| ctx.registries().blocks.parse_state(name).expect("stronghold block");
        Self {
            bricks: s("minecraft:stone_bricks"),
            cave_air: s("minecraft:cave_air"),
            brick_slab: s("minecraft:stone_brick_slab"),
            smooth_slab: s("minecraft:smooth_stone_slab"),
            smooth_double: s("minecraft:smooth_stone_slab[type=double]"),
            wall_torch: s("minecraft:wall_torch"),
            torch: s("minecraft:torch"),
            planks: s("minecraft:oak_planks"),
            bookshelf: s("minecraft:bookshelf"),
            cobweb: s("minecraft:cobweb"),
            fence: s("minecraft:oak_fence"),
            ladder: s("minecraft:ladder"),
            lava: s("minecraft:lava"),
            water: s("minecraft:water"),
            bars: s("minecraft:iron_bars"),
            brick_stairs: s("minecraft:stone_brick_stairs"),
            frame: s("minecraft:end_portal_frame"),
            portal: s("minecraft:end_portal"),
            spawner: s("minecraft:spawner"),
            iron_door: s("minecraft:iron_door"),
            oak_door: s("minecraft:oak_door"),
            button: s("minecraft:stone_button"),
            cobblestone: s("minecraft:cobblestone"),
            cobble_stairs: s("minecraft:cobblestone_stairs"),
        }
    }
}

/// `StrongholdPieces.SmoothStoneSelector`.
struct SmoothStone {
    bricks: BlockStateId,
    cracked: BlockStateId,
    mossy: BlockStateId,
    infested: BlockStateId,
    cave_air: BlockStateId,
}

impl SmoothStone {
    fn load(ctx: &Ctx) -> Self {
        let s = |name: &str| ctx.registries().blocks.parse_state(name).expect("stronghold block");
        Self {
            bricks: s("minecraft:stone_bricks"),
            cracked: s("minecraft:cracked_stone_bricks"),
            mossy: s("minecraft:mossy_stone_bricks"),
            infested: s("minecraft:infested_stone_bricks"),
            cave_air: s("minecraft:cave_air"),
        }
    }
}

impl BlockSelector for SmoothStone {
    fn next(&mut self, _ctx: &Ctx, random: &mut WorldgenRandom, _x: i32, _y: i32, _z: i32, edge: bool) -> BlockStateId {
        if !edge {
            return self.cave_air;
        }
        let selection = random.next_f32();
        if selection < 0.2 {
            self.cracked
        } else if selection < 0.5 {
            self.mossy
        } else if selection < 0.55 {
            self.infested
        } else {
            self.bricks
        }
    }
}

/// `StrongholdPiece.generateSmallDoor`.
fn small_door(p: &PieceBase, ctx: &mut Ctx, b: &B, chunk_bb: &BoundingBox, door: Door, x: i32, y: i32, z: i32) {
    let bars = |ctx: &Ctx, sides: &[&str]| sides.iter().fold(b.bars, |s, side| ctx.with(s, side, "true"));
    match door {
        Door::Opening => p.generate_box(ctx, chunk_bb, x, y, z, x + 2, y + 2, z, b.cave_air, b.cave_air, false),
        Door::WoodDoor | Door::IronDoor => {
            for (dx, dy) in [(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (2, 1), (2, 0)] {
                p.place_block(ctx, b.bricks, x + dx, y + dy, z, chunk_bb);
            }
            let door_block = if door == Door::WoodDoor { b.oak_door } else { b.iron_door };
            p.place_block(ctx, door_block, x + 1, y, z, chunk_bb);
            let upper = ctx.with(door_block, "half", "upper");
            p.place_block(ctx, upper, x + 1, y + 1, z, chunk_bb);
            if door == Door::IronDoor {
                let north = ctx.with(b.button, "facing", "north");
                p.place_block(ctx, north, x + 2, y + 1, z + 1, chunk_bb);
                let south = ctx.with(b.button, "facing", "south");
                p.place_block(ctx, south, x + 2, y + 1, z - 1, chunk_bb);
            }
        }
        Door::Grates => {
            p.place_block(ctx, b.cave_air, x + 1, y, z, chunk_bb);
            p.place_block(ctx, b.cave_air, x + 1, y + 1, z, chunk_bb);
            let west = bars(ctx, &["west"]);
            p.place_block(ctx, west, x, y, z, chunk_bb);
            p.place_block(ctx, west, x, y + 1, z, chunk_bb);
            let east_west = bars(ctx, &["east", "west"]);
            p.place_block(ctx, east_west, x, y + 2, z, chunk_bb);
            p.place_block(ctx, east_west, x + 1, y + 2, z, chunk_bb);
            p.place_block(ctx, east_west, x + 2, y + 2, z, chunk_bb);
            let east = bars(ctx, &["east"]);
            p.place_block(ctx, east, x + 2, y + 1, z, chunk_bb);
            p.place_block(ctx, east, x + 2, y, z, chunk_bb);
        }
    }
}

impl Piece for StrongholdPiece {
    fn base(&self) -> &PieceBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PieceBase {
        &mut self.base
    }

    fn type_name(&self) -> &'static str {
        match self.kind {
            Kind::Start => "minecraft:shstart",
            Kind::ChestCorridor { .. } => "minecraft:shcc",
            Kind::FillerCorridor { .. } => "minecraft:shfc",
            Kind::FiveCrossing { .. } => "minecraft:sh5c",
            Kind::LeftTurn => "minecraft:shlt",
            Kind::Library { .. } => "minecraft:shli",
            Kind::PortalRoom { .. } => "minecraft:shpr",
            Kind::PrisonHall => "minecraft:shph",
            Kind::RightTurn => "minecraft:shrt",
            Kind::RoomCrossing { .. } => "minecraft:shrc",
            Kind::StairsDown => "minecraft:shsd",
            Kind::Straight { .. } => "minecraft:shs",
            Kind::StraightStairsDown => "minecraft:shssd",
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn post_process(&mut self, ctx: &mut Ctx, random: &mut WorldgenRandom, chunk_bb: &BoundingBox, _chunk: ChunkPos, _reference: BlockPos) {
        let b = B::load(ctx);
        let mut stone = SmoothStone::load(ctx);
        let p = self.base.clone();
        let bb = chunk_bb;
        let door = self.entry_door;
        let bx = |ctx: &mut Ctx, x0: i32, y0: i32, z0: i32, x1: i32, y1: i32, z1: i32, s: BlockStateId| p.generate_box(ctx, bb, x0, y0, z0, x1, y1, z1, s, s, false);
        let torch = |ctx: &Ctx, facing: &str| ctx.with(b.wall_torch, "facing", facing);
        match &mut self.kind {
            Kind::Start | Kind::StairsDown => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 4, 10, 4, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 7, 0);
                small_door(&p, ctx, &b, bb, Door::Opening, 1, 1, 4);
                let steps: [(BlockStateId, i32, i32, i32); 17] = [
                    (b.bricks, 2, 6, 1),
                    (b.bricks, 1, 5, 1),
                    (b.smooth_slab, 1, 6, 1),
                    (b.bricks, 1, 5, 2),
                    (b.bricks, 1, 4, 3),
                    (b.smooth_slab, 1, 5, 3),
                    (b.bricks, 2, 4, 3),
                    (b.bricks, 3, 3, 3),
                    (b.smooth_slab, 3, 4, 3),
                    (b.bricks, 3, 3, 2),
                    (b.bricks, 3, 2, 1),
                    (b.smooth_slab, 3, 3, 1),
                    (b.bricks, 2, 2, 1),
                    (b.bricks, 1, 1, 1),
                    (b.smooth_slab, 1, 2, 1),
                    (b.bricks, 1, 1, 2),
                    (b.smooth_slab, 1, 1, 3),
                ];
                for (state, x, y, z) in steps {
                    p.place_block(ctx, state, x, y, z, bb);
                }
            }
            Kind::ChestCorridor { placed_chest } => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 4, 4, 6, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 1, 0);
                small_door(&p, ctx, &b, bb, Door::Opening, 1, 1, 6);
                bx(ctx, 3, 1, 2, 3, 1, 4, b.bricks);
                p.place_block(ctx, b.brick_slab, 3, 1, 1, bb);
                p.place_block(ctx, b.brick_slab, 3, 1, 5, bb);
                p.place_block(ctx, b.brick_slab, 3, 2, 2, bb);
                p.place_block(ctx, b.brick_slab, 3, 2, 4, bb);
                for z in 2..=4 {
                    p.place_block(ctx, b.brick_slab, 2, 1, z, bb);
                }
                if !*placed_chest && bb.is_inside(p.world_pos(3, 2, 3)) {
                    *placed_chest = true;
                    p.create_chest(ctx, bb, random, 3, 2, 3, "minecraft:chests/stronghold_corridor");
                }
            }
            Kind::FillerCorridor { steps } => {
                for i in 0..*steps {
                    for x in 0..=4 {
                        p.place_block(ctx, b.bricks, x, 0, i, bb);
                    }
                    for y in 1..=3 {
                        p.place_block(ctx, b.bricks, 0, y, i, bb);
                        p.place_block(ctx, b.cave_air, 1, y, i, bb);
                        p.place_block(ctx, b.cave_air, 2, y, i, bb);
                        p.place_block(ctx, b.cave_air, 3, y, i, bb);
                        p.place_block(ctx, b.bricks, 4, y, i, bb);
                    }
                    for x in 0..=4 {
                        p.place_block(ctx, b.bricks, x, 4, i, bb);
                    }
                }
            }
            Kind::FiveCrossing { left_low, left_high, right_low, right_high } => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 9, 8, 10, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 4, 3, 0);
                if *left_low {
                    bx(ctx, 0, 3, 1, 0, 5, 3, b.cave_air);
                }
                if *right_low {
                    bx(ctx, 9, 3, 1, 9, 5, 3, b.cave_air);
                }
                if *left_high {
                    bx(ctx, 0, 5, 7, 0, 7, 9, b.cave_air);
                }
                if *right_high {
                    bx(ctx, 9, 5, 7, 9, 7, 9, b.cave_air);
                }
                bx(ctx, 5, 1, 10, 7, 3, 10, b.cave_air);
                p.generate_box_with(ctx, bb, 1, 2, 1, 8, 2, 6, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 1, 5, 4, 4, 9, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 8, 1, 5, 8, 4, 9, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 1, 4, 7, 3, 4, 9, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 1, 3, 5, 3, 3, 6, false, random, &mut stone);
                bx(ctx, 1, 3, 4, 3, 3, 4, b.smooth_slab);
                bx(ctx, 1, 4, 6, 3, 4, 6, b.smooth_slab);
                p.generate_box_with(ctx, bb, 5, 1, 7, 7, 1, 8, false, random, &mut stone);
                bx(ctx, 5, 1, 9, 7, 1, 9, b.smooth_slab);
                bx(ctx, 5, 2, 7, 7, 2, 7, b.smooth_slab);
                bx(ctx, 4, 5, 7, 4, 5, 9, b.smooth_slab);
                bx(ctx, 8, 5, 7, 8, 5, 9, b.smooth_slab);
                bx(ctx, 5, 5, 7, 7, 5, 9, b.smooth_double);
                let t = torch(ctx, "south");
                p.place_block(ctx, t, 6, 5, 6, bb);
            }
            Kind::LeftTurn | Kind::RightTurn => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 4, 4, 4, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 1, 0);
                let north_or_east = matches!(p.orientation(), Some(Direction::North | Direction::East));
                // Facing north or east, a left turn opens at x = 0 and a
                // right turn at x = 4; facing south or west they swap.
                let west_side = (self.kind == Kind::LeftTurn) == north_or_east;
                if west_side {
                    bx(ctx, 0, 1, 1, 0, 3, 3, b.cave_air);
                } else {
                    bx(ctx, 4, 1, 1, 4, 3, 3, b.cave_air);
                }
            }
            Kind::Library { tall } => {
                let tall = *tall;
                let height = if tall { 11 } else { 6 };
                p.generate_box_with(ctx, bb, 0, 0, 0, 13, height - 1, 14, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 4, 1, 0);
                p.generate_maybe_box(ctx, bb, random, 0.07, 2, 1, 1, 11, 4, 13, b.cobweb, b.cobweb, false, false);
                let east_torch = torch(ctx, "east");
                let west_torch = torch(ctx, "west");
                for d in 1..=13 {
                    if (d - 1) % 4 == 0 {
                        bx(ctx, 1, 1, d, 1, 4, d, b.planks);
                        bx(ctx, 12, 1, d, 12, 4, d, b.planks);
                        p.place_block(ctx, east_torch, 2, 3, d, bb);
                        p.place_block(ctx, west_torch, 11, 3, d, bb);
                        if tall {
                            bx(ctx, 1, 6, d, 1, 9, d, b.planks);
                            bx(ctx, 12, 6, d, 12, 9, d, b.planks);
                        }
                    } else {
                        bx(ctx, 1, 1, d, 1, 4, d, b.bookshelf);
                        bx(ctx, 12, 1, d, 12, 4, d, b.bookshelf);
                        if tall {
                            bx(ctx, 1, 6, d, 1, 9, d, b.bookshelf);
                            bx(ctx, 12, 6, d, 12, 9, d, b.bookshelf);
                        }
                    }
                }
                for d in (3..12).step_by(2) {
                    bx(ctx, 3, 1, d, 4, 3, d, b.bookshelf);
                    bx(ctx, 6, 1, d, 7, 3, d, b.bookshelf);
                    bx(ctx, 9, 1, d, 10, 3, d, b.bookshelf);
                }
                if tall {
                    bx(ctx, 1, 5, 1, 3, 5, 13, b.planks);
                    bx(ctx, 10, 5, 1, 12, 5, 13, b.planks);
                    bx(ctx, 4, 5, 1, 9, 5, 2, b.planks);
                    bx(ctx, 4, 5, 12, 9, 5, 13, b.planks);
                    p.place_block(ctx, b.planks, 9, 5, 11, bb);
                    p.place_block(ctx, b.planks, 8, 5, 11, bb);
                    p.place_block(ctx, b.planks, 9, 5, 10, bb);
                    let fence = |ctx: &Ctx, sides: &[&str]| sides.iter().fold(b.fence, |s, side| ctx.with(s, side, "true"));
                    let we = fence(ctx, &["west", "east"]);
                    let ns = fence(ctx, &["north", "south"]);
                    bx(ctx, 3, 6, 3, 3, 6, 11, ns);
                    bx(ctx, 10, 6, 3, 10, 6, 9, ns);
                    bx(ctx, 4, 6, 2, 9, 6, 2, we);
                    bx(ctx, 4, 6, 12, 7, 6, 12, we);
                    let ne = fence(ctx, &["north", "east"]);
                    let se = fence(ctx, &["south", "east"]);
                    let nw = fence(ctx, &["north", "west"]);
                    let sw = fence(ctx, &["south", "west"]);
                    p.place_block(ctx, ne, 3, 6, 2, bb);
                    p.place_block(ctx, se, 3, 6, 12, bb);
                    p.place_block(ctx, nw, 10, 6, 2, bb);
                    for i in 0..=2 {
                        p.place_block(ctx, sw, 8 + i, 6, 12 - i, bb);
                        if i != 2 {
                            p.place_block(ctx, ne, 8 + i, 6, 11 - i, bb);
                        }
                    }
                    let ladder = ctx.with(b.ladder, "facing", "south");
                    for y in 1..=7 {
                        p.place_block(ctx, ladder, 10, y, 13, bb);
                    }
                    let e = fence(ctx, &["east"]);
                    let w = fence(ctx, &["west"]);
                    p.place_block(ctx, e, 6, 9, 7, bb);
                    p.place_block(ctx, w, 7, 9, 7, bb);
                    p.place_block(ctx, e, 6, 8, 7, bb);
                    p.place_block(ctx, w, 7, 8, 7, bb);
                    let nswe = fence(ctx, &["north", "south", "west", "east"]);
                    p.place_block(ctx, nswe, 6, 7, 7, bb);
                    p.place_block(ctx, nswe, 7, 7, 7, bb);
                    p.place_block(ctx, e, 5, 7, 7, bb);
                    p.place_block(ctx, w, 8, 7, 7, bb);
                    let en = ctx.with(e, "north", "true");
                    let es = ctx.with(e, "south", "true");
                    let wn = ctx.with(w, "north", "true");
                    let ws = ctx.with(w, "south", "true");
                    p.place_block(ctx, en, 6, 7, 6, bb);
                    p.place_block(ctx, es, 6, 7, 8, bb);
                    p.place_block(ctx, wn, 7, 7, 6, bb);
                    p.place_block(ctx, ws, 7, 7, 8, bb);
                    for (x, z) in [(5, 7), (8, 7), (6, 6), (6, 8), (7, 6), (7, 8)] {
                        p.place_block(ctx, b.torch, x, 8, z, bb);
                    }
                }
                p.create_chest(ctx, bb, random, 3, 3, 5, "minecraft:chests/stronghold_library");
                if tall {
                    p.place_block(ctx, b.cave_air, 12, 9, 1, bb);
                    p.create_chest(ctx, bb, random, 12, 8, 1, "minecraft:chests/stronghold_library");
                }
            }
            Kind::PortalRoom { placed_spawner } => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 10, 7, 15, false, random, &mut stone);
                small_door(&p, ctx, &b, bb, Door::Grates, 4, 1, 0);
                p.generate_box_with(ctx, bb, 1, 6, 1, 1, 6, 14, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 9, 6, 1, 9, 6, 14, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 2, 6, 1, 8, 6, 2, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 2, 6, 14, 8, 6, 14, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 1, 1, 1, 2, 1, 4, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 8, 1, 1, 9, 1, 4, false, random, &mut stone);
                bx(ctx, 1, 1, 1, 1, 1, 3, b.lava);
                bx(ctx, 9, 1, 1, 9, 1, 3, b.lava);
                p.generate_box_with(ctx, bb, 3, 1, 8, 7, 1, 12, false, random, &mut stone);
                bx(ctx, 4, 1, 9, 6, 1, 11, b.lava);
                let ns_bars = ctx.with(ctx.with(b.bars, "north", "true"), "south", "true");
                let we_bars = ctx.with(ctx.with(b.bars, "west", "true"), "east", "true");
                for z in (3..14).step_by(2) {
                    bx(ctx, 0, 3, z, 0, 4, z, ns_bars);
                    bx(ctx, 10, 3, z, 10, 4, z, ns_bars);
                }
                for x in (2..9).step_by(2) {
                    bx(ctx, x, 3, 15, x, 4, 15, we_bars);
                }
                let stairs = ctx.with(b.brick_stairs, "facing", "north");
                p.generate_box_with(ctx, bb, 4, 1, 5, 6, 1, 7, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 2, 6, 6, 2, 7, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 3, 7, 6, 3, 7, false, random, &mut stone);
                for x in 4..=6 {
                    p.place_block(ctx, stairs, x, 1, 4, bb);
                    p.place_block(ctx, stairs, x, 2, 5, bb);
                    p.place_block(ctx, stairs, x, 3, 6, bb);
                }
                let mut eyes = [false; 12];
                let mut all_eyes = true;
                for eye in &mut eyes {
                    *eye = random.next_f32() > 0.9;
                    all_eyes &= *eye;
                }
                let frames: [(&str, i32, i32); 12] = [
                    ("north", 4, 8),
                    ("north", 5, 8),
                    ("north", 6, 8),
                    ("south", 4, 12),
                    ("south", 5, 12),
                    ("south", 6, 12),
                    ("east", 3, 9),
                    ("east", 3, 10),
                    ("east", 3, 11),
                    ("west", 7, 9),
                    ("west", 7, 10),
                    ("west", 7, 11),
                ];
                for ((facing, x, z), eye) in frames.into_iter().zip(eyes) {
                    let frame = ctx.with(ctx.with(b.frame, "facing", facing), "eye", if eye { "true" } else { "false" });
                    p.place_block(ctx, frame, x, 3, z, bb);
                }
                if all_eyes {
                    for z in 9..=11 {
                        for x in 4..=6 {
                            p.place_block(ctx, b.portal, x, 3, z, bb);
                        }
                    }
                }
                if !*placed_spawner {
                    let pos = p.world_pos(5, 3, 6);
                    if bb.is_inside(pos) {
                        *placed_spawner = true;
                        ctx.set_block(pos, b.spawner);
                        ctx.region.set_spawner_entity(pos.x, pos.y, pos.z, "minecraft:silverfish");
                    }
                }
            }
            Kind::PrisonHall => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 8, 4, 10, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 1, 0);
                bx(ctx, 1, 1, 10, 3, 3, 10, b.cave_air);
                p.generate_box_with(ctx, bb, 4, 1, 1, 4, 3, 1, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 1, 3, 4, 3, 3, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 1, 7, 4, 3, 7, false, random, &mut stone);
                p.generate_box_with(ctx, bb, 4, 1, 9, 4, 3, 9, false, random, &mut stone);
                let bars = |ctx: &Ctx, sides: &[&str]| sides.iter().fold(b.bars, |s, side| ctx.with(s, side, "true"));
                let ns = bars(ctx, &["north", "south"]);
                let nse = bars(ctx, &["north", "south", "east"]);
                let we = bars(ctx, &["west", "east"]);
                for y in 1..=3 {
                    p.place_block(ctx, ns, 4, y, 4, bb);
                    p.place_block(ctx, nse, 4, y, 5, bb);
                    p.place_block(ctx, ns, 4, y, 6, bb);
                    p.place_block(ctx, we, 5, y, 5, bb);
                    p.place_block(ctx, we, 6, y, 5, bb);
                    p.place_block(ctx, we, 7, y, 5, bb);
                }
                p.place_block(ctx, ns, 4, 3, 2, bb);
                p.place_block(ctx, ns, 4, 3, 8, bb);
                let bottom = ctx.with(b.iron_door, "facing", "west");
                let top = ctx.with(bottom, "half", "upper");
                p.place_block(ctx, bottom, 4, 1, 2, bb);
                p.place_block(ctx, top, 4, 2, 2, bb);
                p.place_block(ctx, bottom, 4, 1, 8, bb);
                p.place_block(ctx, top, 4, 2, 8, bb);
            }
            Kind::RoomCrossing { room } => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 10, 6, 10, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 4, 1, 0);
                bx(ctx, 4, 1, 10, 6, 3, 10, b.cave_air);
                bx(ctx, 0, 1, 4, 0, 3, 6, b.cave_air);
                bx(ctx, 10, 1, 4, 10, 3, 6, b.cave_air);
                match *room {
                    0 => {
                        p.place_block(ctx, b.bricks, 5, 1, 5, bb);
                        p.place_block(ctx, b.bricks, 5, 2, 5, bb);
                        p.place_block(ctx, b.bricks, 5, 3, 5, bb);
                        for (facing, x, z) in [("west", 4, 5), ("east", 6, 5), ("south", 5, 4), ("north", 5, 6)] {
                            let t = torch(ctx, facing);
                            p.place_block(ctx, t, x, 3, z, bb);
                        }
                        for (x, z) in [(4, 4), (4, 5), (4, 6), (6, 4), (6, 5), (6, 6), (5, 4), (5, 6)] {
                            p.place_block(ctx, b.smooth_slab, x, 1, z, bb);
                        }
                    }
                    1 => {
                        for i in 0..5 {
                            p.place_block(ctx, b.bricks, 3, 1, 3 + i, bb);
                            p.place_block(ctx, b.bricks, 7, 1, 3 + i, bb);
                            p.place_block(ctx, b.bricks, 3 + i, 1, 3, bb);
                            p.place_block(ctx, b.bricks, 3 + i, 1, 7, bb);
                        }
                        p.place_block(ctx, b.bricks, 5, 1, 5, bb);
                        p.place_block(ctx, b.bricks, 5, 2, 5, bb);
                        p.place_block(ctx, b.bricks, 5, 3, 5, bb);
                        p.place_block(ctx, b.water, 5, 4, 5, bb);
                    }
                    2 => {
                        for z in 1..=9 {
                            p.place_block(ctx, b.cobblestone, 1, 3, z, bb);
                            p.place_block(ctx, b.cobblestone, 9, 3, z, bb);
                        }
                        for x in 1..=9 {
                            p.place_block(ctx, b.cobblestone, x, 3, 1, bb);
                            p.place_block(ctx, b.cobblestone, x, 3, 9, bb);
                        }
                        for (x, y, z) in [(5, 1, 4), (5, 1, 6), (5, 3, 4), (5, 3, 6), (4, 1, 5), (6, 1, 5), (4, 3, 5), (6, 3, 5)] {
                            p.place_block(ctx, b.cobblestone, x, y, z, bb);
                        }
                        for y in 1..=3 {
                            p.place_block(ctx, b.cobblestone, 4, y, 4, bb);
                            p.place_block(ctx, b.cobblestone, 6, y, 4, bb);
                            p.place_block(ctx, b.cobblestone, 4, y, 6, bb);
                            p.place_block(ctx, b.cobblestone, 6, y, 6, bb);
                        }
                        p.place_block(ctx, b.wall_torch, 5, 3, 5, bb);
                        for z in 2..=8 {
                            p.place_block(ctx, b.planks, 2, 3, z, bb);
                            p.place_block(ctx, b.planks, 3, 3, z, bb);
                            if z <= 3 || z >= 7 {
                                p.place_block(ctx, b.planks, 4, 3, z, bb);
                                p.place_block(ctx, b.planks, 5, 3, z, bb);
                                p.place_block(ctx, b.planks, 6, 3, z, bb);
                            }
                            p.place_block(ctx, b.planks, 7, 3, z, bb);
                            p.place_block(ctx, b.planks, 8, 3, z, bb);
                        }
                        let ladder = ctx.with(b.ladder, "facing", "west");
                        for y in 1..=3 {
                            p.place_block(ctx, ladder, 9, y, 3, bb);
                        }
                        p.create_chest(ctx, bb, random, 3, 4, 8, "minecraft:chests/stronghold_crossing");
                    }
                    _ => {}
                }
            }
            Kind::Straight { left, right } => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 4, 4, 6, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 1, 0);
                small_door(&p, ctx, &b, bb, Door::Opening, 1, 1, 6);
                let east = torch(ctx, "east");
                let west = torch(ctx, "west");
                p.maybe_generate_block(ctx, bb, random, 0.1, 1, 2, 1, east);
                p.maybe_generate_block(ctx, bb, random, 0.1, 3, 2, 1, west);
                p.maybe_generate_block(ctx, bb, random, 0.1, 1, 2, 5, east);
                p.maybe_generate_block(ctx, bb, random, 0.1, 3, 2, 5, west);
                if *left {
                    bx(ctx, 0, 1, 2, 0, 3, 4, b.cave_air);
                }
                if *right {
                    bx(ctx, 4, 1, 2, 4, 3, 4, b.cave_air);
                }
            }
            Kind::StraightStairsDown => {
                p.generate_box_with(ctx, bb, 0, 0, 0, 4, 10, 7, true, random, &mut stone);
                small_door(&p, ctx, &b, bb, door, 1, 7, 0);
                small_door(&p, ctx, &b, bb, Door::Opening, 1, 1, 7);
                let stairs = ctx.with(b.cobble_stairs, "facing", "south");
                for i in 0..6 {
                    p.place_block(ctx, stairs, 1, 6 - i, 1 + i, bb);
                    p.place_block(ctx, stairs, 2, 6 - i, 1 + i, bb);
                    p.place_block(ctx, stairs, 3, 6 - i, 1 + i, bb);
                    if i < 5 {
                        p.place_block(ctx, b.bricks, 1, 5 - i, 1 + i, bb);
                        p.place_block(ctx, b.bricks, 2, 5 - i, 1 + i, bb);
                        p.place_block(ctx, b.bricks, 3, 5 - i, 1 + i, bb);
                    }
                }
            }
        }
    }
}
